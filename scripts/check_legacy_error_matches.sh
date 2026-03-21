#!/usr/bin/env bash
# check_legacy_error_matches.sh
#
# Scans crates/ui/web-api/src/routes/ for legacy error-match patterns that
# should be replaced with `?` propagation through ApiError.
#
# Patterns detected:
#   PATTERN1: match <expr>.current_context() { ... }
#   PATTERN2: matches!(<expr>.current_context(), ...)
#
# Behaviour:
#   - If crates/ui/web-api/MIGRATION_IN_PROGRESS exists → warning mode (exit 0,
#     prints matches with a warning header).
#   - Otherwise → hard-fail mode (exit 1 if any matches found).
#
# Usage:
#   ./scripts/check_legacy_error_matches.sh
#
# Exit codes:
#   0  No violations (or warning mode)
#   1  Violations found (hard-fail mode only)

set -euo pipefail

ROUTES_DIR="crates/ui/web-api/src/routes"
MARKER="crates/ui/web-api/MIGRATION_IN_PROGRESS"

PATTERN1='match[[:space:]]+[[:alpha:]_][[:alnum:]_]*\.current_context\(\)'
PATTERN2='matches!\(.*\.current_context\(\)'

# Collect matches from both patterns.
MATCHES=$(
  grep -rEn "$PATTERN1" "$ROUTES_DIR" 2>/dev/null || true
  grep -rEn "$PATTERN2" "$ROUTES_DIR" 2>/dev/null || true
)

if [ -z "$MATCHES" ]; then
  echo "check_legacy_error_matches: OK — no legacy patterns found."
  exit 0
fi

COUNT=$(echo "$MATCHES" | wc -l | tr -d ' ')

if [ -f "$MARKER" ]; then
  echo "check_legacy_error_matches: WARNING — migration in progress."
  echo "  Found $COUNT legacy error-match pattern(s) (warning mode — not failing):"
  echo "$MATCHES" | sed 's/^/  /'
  exit 0
else
  echo "check_legacy_error_matches: FAIL — $COUNT legacy error-match pattern(s) found."
  echo "  All match blocks must be replaced with ApiError '?' propagation."
  echo ""
  echo "$MATCHES" | sed 's/^/  /'
  exit 1
fi
