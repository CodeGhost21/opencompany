#!/bin/sh
# Run the desktop shell against a live console dev server.
#
# ## Why this is a script and not a `beforeDevCommand`
#
# A debug build of the shell loads `devUrl` (`http://localhost:5173`) rather
# than the embedded bundle, so without a dev server the window is blank. The
# obvious fix is `build.beforeDevCommand` in `src-tauri/tauri.conf.json`, and
# it is wrong: the Tauri CLI runs that hook from a directory it *derives* by
# scanning for a `package.json`, and which one it picks is not stable — on a
# macOS checkout it lands in `frontend/`, on CI's runner it landed in
# `vendor/openhuman/`. No relative path is correct from both, which is why
# those hooks are deliberately empty and why `ci.yml` packages from two
# different working directories to keep them that way (issue #616).
#
# A script has the one thing the hook does not: it knows where it is. Every
# path below is absolute, derived from `$0`, so nothing here depends on the
# directory it was invoked from.
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "${SCRIPT_DIR}/.." && pwd)

# The port `devUrl` names. Not configurable here on purpose: a dev server on
# some other port is a window that loads nothing, and silently picking a
# different one is how that becomes a mystery instead of an error.
DEV_PORT=5173
DEV_URL="http://localhost:${DEV_PORT}"

usage() {
    cat >&2 <<'EOF'
Usage: ./scripts/desktop-dev.sh

Starts the console dev server (if one is not already up) and runs the desktop
shell against it. Ctrl-C stops both.

Environment:
  OPENCOMPANY_DATA_DIR   Instance data root. Point it at a scratch directory to
                         leave your installed application's data alone:

    OPENCOMPANY_DATA_DIR=$PWD/target/desktop-dev ./scripts/desktop-dev.sh
EOF
}

if [ "$#" -ne 0 ]; then
    usage
    exit 2
fi

# `curl` is only used to ask whether something is listening, so a non-2xx
# answer is still a yes.
serving() {
    curl -sS -o /dev/null --max-time 2 "${DEV_URL}/" 2>/dev/null
}

DEV_SERVER_PID=""

cleanup() {
    # Only ever the server this script started. A dev server that was already
    # running belongs to whoever started it, and killing it would close
    # someone else's terminal out from under them.
    if [ -n "${DEV_SERVER_PID}" ]; then
        kill "${DEV_SERVER_PID}" 2>/dev/null || true
        wait "${DEV_SERVER_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# Whichever package manager this checkout has. `pnpm` first because that is
# what `pnpm-workspace.yaml` declares, `npm` second because that is what CI
# installs with — and either can run the `dev` script whichever one populated
# `node_modules`.
if command -v pnpm >/dev/null 2>&1; then
    PACKAGE_MANAGER=pnpm
elif command -v npm >/dev/null 2>&1; then
    PACKAGE_MANAGER=npm
else
    echo "desktop-dev: neither pnpm nor npm is on PATH" >&2
    exit 1
fi

if serving; then
    echo "desktop-dev: reusing the console dev server already on ${DEV_URL}"
else
    if [ ! -d "${REPO_ROOT}/frontend/node_modules" ]; then
        echo "desktop-dev: install the console's dependencies first:" >&2
        echo "    ${PACKAGE_MANAGER} --dir '${REPO_ROOT}/frontend' install" >&2
        exit 1
    fi
    echo "desktop-dev: starting the console dev server on ${DEV_URL}"
    # `cd` in a subshell rather than a `--dir`/`--prefix` flag, which the two
    # package managers spell differently.
    (cd "${REPO_ROOT}/frontend" && exec "${PACKAGE_MANAGER}" run dev) &
    DEV_SERVER_PID=$!

    # Waited for rather than slept past: the shell loads `devUrl` the moment it
    # opens its window, and a race here is exactly the blank screen this script
    # exists to prevent.
    waited=0
    until serving; do
        if ! kill -0 "${DEV_SERVER_PID}" 2>/dev/null; then
            echo "desktop-dev: the console dev server exited" >&2
            exit 1
        fi
        waited=$((waited + 1))
        if [ "${waited}" -gt 60 ]; then
            echo "desktop-dev: nothing answered on ${DEV_URL} after 30s" >&2
            exit 1
        fi
        sleep 0.5
    done
fi

# The CLI from `frontend/node_modules`, as `ci.yml` uses, falling back to a
# `cargo install`ed one. Run from `src-tauri` so the CLI finds this project:
# it searches *subfolders* of the working directory, so from `frontend/` it
# would pick the console wrapper in `frontend/src-tauri/` instead — a different
# application that happens to share this one's `productName`.
TAURI_CLI="${REPO_ROOT}/frontend/node_modules/.bin/tauri"
cd "${REPO_ROOT}/src-tauri"
if [ -x "${TAURI_CLI}" ]; then
    "${TAURI_CLI}" dev
else
    cargo tauri dev
fi
