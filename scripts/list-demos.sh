#!/bin/sh
# List friendly demo aliases and all company directory names.
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "${SCRIPT_DIR}/.." && pwd)

cat <<'EOF'
Friendly names:
  marketing     marketing_agency
  software      software_company
  fund          venture_capital
  studio        venture_studio
  accelerator   startup_accelerator
  law           law_firm
  accounting    accounting_firm
  support       customer_support
  signals       signals_opportunity_studio

Company directory names:
EOF

for manifest in "${REPO_ROOT}"/companies/*/company.toml "${REPO_ROOT}"/companies/*/agents.toml; do
    [ -f "$manifest" ] || continue
    basename "$(dirname "$manifest")"
done | sort -u | sed 's/^/  /'
