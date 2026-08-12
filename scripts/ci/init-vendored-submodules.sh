#!/bin/sh
#
# Initialize the vendored openhuman crate's own submodules. Issue #592.
#
# THE ONE PLACE THE LIST LIVES. Adding a submodule under `vendor/openhuman`?
# Add it here, nowhere else. Do not re-inline these commands into a workflow.
#
# This file exists because the block below was copied into four jobs of
# `ci.yml` and a fifth in `release.yml`, ~340 lines apart in a file long enough
# that a reader edits one copy and never learns the others exist. The failure
# that shape produces is the one issue #555 was filed for: a lane quietly not
# doing what its siblings do, found late and by accident. It had already
# happened by the time #592 was written — `release.yml` was missing
# `vendor/tinyhumans-sdk` and the nested `tinycortex` init entirely, and had
# gone unnoticed only because that workflow is `workflow_dispatch`-only and had
# never run.
#
# WHY THESE CRATES MUST BE ON DISK EVEN FOR A DEFAULT BUILD
#
# `openhuman_core` is an optional path dependency, but Cargo reads every path
# dependency's manifest during resolution regardless of feature selection. So
# the vendored crate's own path deps must resolve even for the default
# (offline) build where the `openhuman` feature is off — a missing checkout
# fails the DEFAULT build, not just `--features openhuman`.
#
# `tinyhumans-sdk` is the case that catches people out: it is NOT one of the
# `[patch]` targets. The vendored openhuman crate consumes it as a plain
# unconditional path dependency because it is unpublished (issue #499). Absent
# from this list, every lane breaks — which is exactly how `release.yml` came
# to be latently broken.
#
# `tinycortex` in turn declares its own `tinyagents` path dependency, so its
# manifest must resolve too. Harmless for the default build; required for any
# `--features openhuman` build.
#
# WHY TARGETED RATHER THAN `--recursive`
#
# A plain `--recursive` over `vendor/openhuman` additionally drags in the
# desktop-only `tauri-cef` tree — a heavy CEF checkout that nothing in these
# builds compiles.
#
# WHAT THIS DELIBERATELY DOES NOT DO
#
# It does not initialize the TOP-LEVEL submodules. Each caller chooses its own
# checkout strategy (`submodules: true` on `actions/checkout`, or an explicit
# init), and quietly changing that from in here would be a surprise. This
# script assumes `vendor/openhuman` is already checked out and only fills in
# the crates nested beneath it.
#
# Idempotent: safe to run repeatedly, and a no-op once the submodules are
# present at their pinned commits.
#
# Usage:
#   scripts/ci/init-vendored-submodules.sh
#
# Runs from any working directory — it resolves the repository root itself.

set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "${SCRIPT_DIR}/../.." && pwd)
cd "${REPO_ROOT}"

# Without this, git emits a bare "not a git repository" that says nothing about
# the actual mistake, which is a caller that never checked out the top level.
if [ ! -f vendor/openhuman/.gitmodules ]; then
  echo "init-vendored-submodules: vendor/openhuman/.gitmodules is missing, so" >&2
  echo "vendor/openhuman itself was never checked out. This script initializes" >&2
  echo "the crates NESTED under it and cannot create it." >&2
  echo >&2
  echo "In a workflow: pass \`submodules: true\` to actions/checkout." >&2
  echo "Locally: git submodule update --init vendor/openhuman" >&2
  exit 1
fi

git -C vendor/openhuman submodule update --init --depth 1 \
  vendor/tinyagents vendor/tinybus vendor/tinyflows vendor/tinycortex \
  vendor/tinydocs vendor/tinymemory vendor/tinywallet \
  vendor/tinyjuice vendor/tinychannels vendor/tinyplace vendor/tinyhumans-sdk
git -C vendor/openhuman/vendor/tinycortex submodule update --init --depth 1 \
  vendor/tinyagents
