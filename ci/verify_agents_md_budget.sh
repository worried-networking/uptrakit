#!/usr/bin/env bash
# verify_agents_md_budget.sh: enforce size budgets on AI-agent guide files.
#
# The root AGENTS.md regrew from 339 to 1944 lines in five months because
# nothing gated its size. This script is the backstop; the primary control is
# the "no code-structure inventory in any AGENTS.md" maintenance rule inside
# AGENTS.md itself.
#
# Budgets:
#   - root AGENTS.md:            <= 500 lines and <= 60 KB
#   - any other */AGENTS.md:     <= 250 lines
#
# Exit codes: 0 = all within budget, 1 = at least one file over budget.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ROOT_MAX_LINES=500
ROOT_MAX_BYTES=61440
SCOPED_MAX_LINES=250

failures=0

check_root() {
  local file="AGENTS.md"
  [[ -f "$file" ]] || return 0
  local lines bytes
  lines=$(wc -l <"$file" | tr -d ' ')
  bytes=$(wc -c <"$file" | tr -d ' ')
  if ((lines > ROOT_MAX_LINES)); then
    echo "verify_agents_md_budget: $file has $lines lines (budget: $ROOT_MAX_LINES)."
    failures=1
  fi
  if ((bytes > ROOT_MAX_BYTES)); then
    echo "verify_agents_md_budget: $file is $bytes bytes (budget: $ROOT_MAX_BYTES)."
    failures=1
  fi
}

check_scoped() {
  local file lines
  while IFS= read -r file; do
    [[ "$file" == "AGENTS.md" ]] && continue
    lines=$(wc -l <"$file" | tr -d ' ')
    if ((lines > SCOPED_MAX_LINES)); then
      echo "verify_agents_md_budget: $file has $lines lines (budget: $SCOPED_MAX_LINES)."
      failures=1
    fi
  done < <(git ls-files '*AGENTS.md' 'AGENTS.md')
}

check_root
check_scoped

if ((failures)); then
  echo "verify_agents_md_budget: over budget. Move detail into docs/ and keep AGENTS.md files as invariants + pointers (see AGENTS.md 'Maintaining this file')."
  exit 1
fi

echo "verify_agents_md_budget: OK"
