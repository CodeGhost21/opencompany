#!/bin/sh
#
# Run the first-run end-to-end lane and assert it actually executed something.
# Issue #1404.
#
# The trap this closes is the one `assert-integration-targets-run.sh` closes for
# Rust targets, in a browser suite. `frontend/test/e2e/company-setup.spec.ts` —
# the only proof first-run company setup works at all — opened with:
#
#   test.skip(left.length > 0, "this company ships with N manifest agents ...")
#
# written for a host serving the wrong company. Once the global baseline began
# merging four undeletable teammates into EVERY company, that guard fired on
# every run, including the right one. Playwright reported `2 skipped`, exited 0,
# and the lane was green over a feature that could not open anywhere in the
# shipped product.
#
# So this checks a NUMBER. The spec's own guard now fails rather than skips, and
# `playwright.config.ts` selects it only in a first-run run — but both of those
# are configuration, and configuration can look right and be vacuous. A count
# cannot.
#
# Usage:
#   scripts/ci/assert-e2e-spec-ran.sh
#
# Run from anywhere; it resolves the repository root itself. It expects a host
# binary at `target/debug/opencompany` (or `PW_HOST_BINARY`), exactly as
# `npm run e2e` does — see `frontend/test/e2e/host.sh`.
#
# Requires `jq` (present on `ubuntu-latest`; `brew install jq` locally).

set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "${SCRIPT_DIR}/../.." && pwd)

if ! command -v jq > /dev/null 2>&1; then
  echo "assert-e2e-spec-ran: jq is required but not installed" >&2
  exit 1
fi

REPORT="${REPO_ROOT}/target/e2e/first-run-report.json"
mkdir -p "$(dirname "${REPORT}")"
rm -f "${REPORT}"

cd "${REPO_ROOT}/frontend"

# `list` keeps the human-readable output in the log; `json` is what this script
# reads. Playwright writes the JSON to PLAYWRIGHT_JSON_OUTPUT_NAME rather than
# stdout when that variable is set, which is what keeps the two from colliding.
set +e
PLAYWRIGHT_JSON_OUTPUT_NAME="${REPORT}" \
  npm run e2e:first-run -- --reporter=list,json
run_status=$?
set -e

if [ ! -f "${REPORT}" ]; then
  cat >&2 << EOF
No JSON report at ${REPORT}.

Playwright did not get far enough to write one, so the run failed before any
test was selected — a host that never bound, a missing binary, or a config
error. Its output is above.
EOF
  exit 1
fi

expected=$(jq -r '.stats.expected // 0' "${REPORT}")
unexpected=$(jq -r '.stats.unexpected // 0' "${REPORT}")
skipped=$(jq -r '.stats.skipped // 0' "${REPORT}")
flaky=$(jq -r '.stats.flaky // 0' "${REPORT}")

echo
echo "first-run lane: ${expected} passed, ${unexpected} failed, ${flaky} flaky, ${skipped} skipped"

if [ "${expected}" -eq 0 ]; then
  cat >&2 << EOF

The first-run lane executed no tests.

A lane that selects nothing reports success having proved nothing, which is the
exact defect this lane was added to fix. Something is wrong with the WIRING, not
with the spec:

  * \`playwright.config.ts\` selects \`company-setup.spec.ts\` only when
    PW_FIRST_RUN=1 — check that \`npm run e2e:first-run\` still sets it;
  * the spec may have been renamed out of the FIRST_RUN_SPEC pattern;
  * every test in it may be skipped. Do NOT restore a \`test.skip\` to make this
    pass — a first-run lane that skips itself is worse than no lane, and is what
    issue #1404 was filed about.
EOF
  exit 1
fi

if [ "${skipped}" -ne 0 ]; then
  cat >&2 << EOF

${skipped} test(s) in the first-run lane skipped themselves.

This lane brings up the host the spec needs, so nothing in it has a legitimate
reason to skip. A skip here is the silent-green shape all over again: fix the
condition, or fail on it, but do not leave it reporting nothing.
EOF
  exit 1
fi

exit "${run_status}"
