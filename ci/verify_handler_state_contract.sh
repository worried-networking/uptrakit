#!/usr/bin/env bash
# Verify that no route handler mixes focused State<SubState> with
# State<Arc<AppState>> in the same function signature.
#
# Scope: crates/ui/web-api/src/routes/ only.
# Middleware is NOT checked by this script (see PLAN-0002 Task 2.5 for
# the explicit grep checks for middleware files).
#
# Exit codes:
#   0 — no violations found
#   1 — at least one handler mixes focused sub-state with full AppState
set -euo pipefail

ROUTES_DIR="crates/ui/web-api/src/routes"
TMP_SIGS=$(mktemp)
TMP_VIOLATIONS=$(mktemp)
trap 'rm -f "$TMP_SIGS" "$TMP_VIOLATIONS"' EXIT

# Step 1: Extract complete async fn signatures into a temp file.
# Each record is: FILENAME:SIGNATURE (one per line, multi-line sigs joined).
# perl joins continuation lines up to the opening '{' so each signature
# is a single record.
find "$ROUTES_DIR" -name '*.rs' -type f -print0 |
    xargs -0 perl -0777 -ne '
    while (/^\s*(?:pub\s+)?async\s+fn\s+\w+.*?[{]/gms) {
      my $sig = $&;
      $sig =~ s/\n/ /g;
      print "$ARGV: $sig\n";
    }
  ' >"$TMP_SIGS"

# Step 2: For each extracted signature, check the XOR violation.
# A violation is a signature containing BOTH State<Arc<AppState>> AND
# any focused State<SubState> (DbState, AuthState, BroadcastState,
# CertState, OidcState).
grep 'State<Arc<AppState>>' "$TMP_SIGS" |
    grep -E 'State<(DbState|AuthState|BroadcastState|CertState|OidcState)>' \
        >"$TMP_VIOLATIONS" || true

# Step 3: Report and exit.
if [ -s "$TMP_VIOLATIONS" ]; then
    echo "ERROR: Handler state contract violations found in routes/:"
    echo "Handlers must use focused State<SubState> OR State<Arc<AppState>>, never both."
    echo ""
    cat "$TMP_VIOLATIONS"
    exit 1
fi

echo "OK: No handler state contract violations in routes/."
exit 0
