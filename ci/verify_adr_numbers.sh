#!/usr/bin/env bash
# verify_adr_numbers.sh: no two ADRs may share a 4-digit number.
#
# The repo landed two 0033-*.md files from parallel branches on 2026-07-29;
# this guard makes that impossible to repeat silently.
#
# Modes:
#   (no args)                    fail on duplicate numbers among tracked docs/adr files
#   --against <rev> [<branch>]   additionally fail when an ADR number ADDED on
#                                <branch> (default HEAD) relative to <rev> already
#                                exists in <rev>'s tree — the collision a rebase or
#                                merge onto <rev> would create.
#
# Pure bash + git on purpose: hooks must enforce this on machines without the
# adrs binary (adrs doctor's ADR012 is the binary-backed equivalent in CI).
# Exit codes: 0 = no collision, 1 = collision or usage/input error.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ADR_DIR="docs/adr"
failures=0

numbers_in() {  # stdin: file paths; stdout: sorted 4-digit prefixes
  sed -nE "s|^$ADR_DIR/([0-9]{4})-.*\.md$|\1|p" | sort
}

tracked_files=$(git ls-files -- "$ADR_DIR" | grep -E "^$ADR_DIR/[0-9]{4}-.*\.md$" || true)
if [[ -z "$tracked_files" ]]; then
  echo "verify_adr_numbers: no numbered ADR files found under $ADR_DIR — refusing to pass vacuously."
  exit 1
fi

while IFS= read -r n; do
  [[ -n "$n" ]] || continue
  echo "verify_adr_numbers: duplicate ADR number $n:"
  printf '%s\n' "$tracked_files" | grep "/$n-" | sed 's/^/  /'
  failures=1
done < <(printf '%s\n' "$tracked_files" | numbers_in | uniq -d)

if [[ "${1:-}" == "--against" ]]; then
  rev="${2:-}"
  if [[ -z "$rev" ]]; then
    echo "verify_adr_numbers: --against requires a revision."
    exit 1
  fi
  branch="${3:-HEAD}"
  base_numbers=$(git ls-tree -r --name-only "$rev" -- "$ADR_DIR" | numbers_in | uniq)
  while IFS= read -r n; do
    [[ -n "$n" ]] || continue
    if printf '%s\n' "$base_numbers" | grep -qx "$n"; then
      echo "verify_adr_numbers: ADR number $n added on $branch already exists in $rev —"
      echo "  rebasing/merging would create a duplicate. Renumber to the next free number."
      failures=1
    fi
  # --no-renames: two independently-added ADRs sharing the standard template
  # boilerplate (Date/Status/Context/Decision/Consequences headers) clear git's
  # default ~50% similarity threshold and get paired as a rename, which
  # --diff-filter=A does not match — silently defeating this exact check.
  done < <(git diff --no-renames --name-only --diff-filter=A "$rev".."$branch" -- "$ADR_DIR" | numbers_in | uniq)
fi

if ((failures)); then
  exit 1
fi
echo "verify_adr_numbers: OK"
