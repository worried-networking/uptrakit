#!/usr/bin/env bash
# verify_no_raw_body_extractors.sh — request bodies must go through the
# Unvalidated<T>/UnvalidatedForm<T>/Validated<T> extractors. Raw Json<T>/Form<T>
# body params, raw Request/Bytes body reads, and unknown FromRequest impls in
# routes/ fail unless allowlisted (frozen shrink-only legacy list).
# See docs/development/coding-standards.md (Request Type Validation) and ADR-0038.
set -euo pipefail

cd "$(dirname "$0")/.."
ROUTES_DIR="crates/ui/web-api/src/routes"
WEB_API_SRC="crates/ui/web-api/src"
ALLOWLIST="ci/verify_no_raw_body_extractors_allowlist.txt"
# Shrink-only ratchet: decrement as Stage 2 converts sites; never increment.
MAX_ALLOWLIST_ENTRIES=34

if ! command -v perl >/dev/null 2>&1; then
  echo "perl is required for verify_no_raw_body_extractors.sh" >&2
  exit 1
fi

TMP_SIGS="$(mktemp)"
trap 'rm -f "$TMP_SIGS"' EXIT

# Extract every async fn signature, flattened to one line, return type stripped.
# Signature extraction is anchored to COLUMN 0: rustfmt (CI-enforced via
# `cargo fmt --all -- --check`) guarantees top-level items start at column 0,
# and every fn inside an inline `mod tests { ... }` block is indented — so
# inline test-module fns can never match, with NO test-cfg parsing at all.
# Dedicated test FILES (`tests.rs` — `#[cfg(test)] mod tests;` submodules
# whose fns sit at column 0) are excluded by filename instead. The pub
# prefix matches every visibility form: pub, pub(crate), pub(super),
# pub(in path). (Brace-counting removal of test modules was tried and
# abandoned: test code legally contains unbalanced braces in string payloads —
# oidc_auth.rs ships a deliberately malformed `br#"{"link_token":..."#` JSON
# body — and truncate-at-first-cfg silently unscans handlers defined AFTER an
# interior test module, e.g. auth.rs refresh/me/confirm_email_change.)
find "$ROUTES_DIR" -name '*.rs' -type f -not -name 'tests.rs' -not -name '*_tests.rs' -print0 |
  xargs -0 perl -0777 -ne '
    while (/(?:^|\n)((?:pub(?:\([^)]*\))?\s+)?async\s+fn\s+\w+.*?\{)/gs) {
      my $sig = $1;
      $sig =~ s/->.*//s;
      $sig =~ s/\n/ /g;
      print "$ARGV\t$sig\n";
    }
  ' >"$TMP_SIGS"

if [ ! -s "$TMP_SIGS" ]; then
  echo "No signatures extracted from $ROUTES_DIR — gate is broken" >&2
  exit 1
fi

# Raw body extraction in param position:
#  - Json< / Form<  (covers Option<Json<...>> via the inner match)
#  - `: Request [,)]` / `: Bytes [,)]` anchored to param TYPE position — a bare
#    "Request" token would match every *Request body type name.
RAW_PATTERN='(Json|Form)[[:space:]]*<|:[[:space:]]*(axum::extract::)?Request[[:space:]]*(<|[,)])|:[[:space:]]*(axum::(body|extract)::)?Bytes[[:space:]]*(<|[,)])'

is_allowlisted() {
  local file="$1" sig="$2" al_class al_path al_regex
  while IFS='|' read -r al_class al_path al_regex; do
    case "$al_class" in ''|'#'*) continue ;; esac
    if [ "$file" = "$al_path" ] && printf '%s' "$sig" | grep -Eq "$al_regex"; then
      return 0
    fi
  done <"$ALLOWLIST"
  return 1
}

violations=0

while IFS=$'\t' read -r file sig; do
  # axum middleware (fn_fn/from_fn signatures carry `next: Next`) passes the body
  # through untouched — the invariant targets handlers that CONSUME bodies.
  if printf '%s' "$sig" | grep -Eq 'Next[[:space:]]*[,)]'; then
    continue
  fi
  if printf '%s' "$sig" | grep -Eq "$RAW_PATTERN"; then
    if ! is_allowlisted "$file" "$sig"; then
      echo "VIOLATION: raw body extraction (use Unvalidated<T>/Validated<T>, or allowlist with justification): $file: $sig" >&2
      violations=1
    fi
  fi
done <"$TMP_SIGS"

# Stale-entry check: every allowlist row must still match a flagged signature.
while IFS='|' read -r al_class al_path al_regex; do
  case "$al_class" in ''|'#'*) continue ;; esac
  hit=0
  while IFS=$'\t' read -r file sig; do
    if [ "$file" = "$al_path" ] && printf '%s' "$sig" | grep -Eq "$al_regex" &&
      printf '%s' "$sig" | grep -Eq "$RAW_PATTERN"; then
      hit=1
      break
    fi
  done <"$TMP_SIGS"
  if [ "$hit" -eq 0 ]; then
    echo "STALE allowlist entry (site converted or renamed — delete the row, decrement MAX_ALLOWLIST_ENTRIES): $al_class|$al_path|$al_regex" >&2
    violations=1
  fi
done <"$ALLOWLIST"

# Shrink-only ratchet.
entry_count=$(grep -cE '^raw_(extractor|body)\|' "$ALLOWLIST" || true)
if [ "$entry_count" -gt "$MAX_ALLOWLIST_ENTRIES" ]; then
  echo "Allowlist grew ($entry_count > $MAX_ALLOWLIST_ENTRIES) — additions prohibited (shrink-only ratchet)" >&2
  violations=1
fi

# Residual check: every raw_extractor (legacy B1) fn must still call .validate()
# in its body span (column-0 fn start to the first column-0 closing brace `}` —
# the same rustfmt-guaranteed column-0 anchoring used above, so a `.validate()`
# living in a nested/test module can never satisfy this check; no test-cfg
# parsing needed).
while IFS='|' read -r al_class al_path al_regex; do
  case "$al_class" in raw_extractor) ;; *) continue ;; esac
  fn_name=$(printf '%s' "$al_regex" | sed -nE 's/.*fn ([a-z_0-9]+).*/\1/p')
  if [ -z "$fn_name" ]; then
    echo "MALFORMED allowlist row (fn-regex must contain 'fn <name>'): $al_class|$al_path|$al_regex" >&2
    violations=1
    continue
  fi
  # Body span = column-0 fn start to the first column-0 closing brace (rustfmt
  # puts a top-level fn's closing `}` at column 0; everything nested is
  # indented). Column-0 anchoring means a `.validate()` inside a test module
  # can never satisfy this check — no test-cfg parsing needed.
  body=$(perl -0777 -ne '
    if (/(?:^|\n)((?:pub(?:\([^)]*\))?\s+)?async\s+fn\s+'"$fn_name"'\b.*?\n\})/s) { print $1; }
  ' "$al_path")
  case "$body" in
  *'.validate('*) ;;
  *)
    echo "RESIDUAL: allowlisted fn $fn_name in $al_path no longer calls .validate() — convert it to Unvalidated<T> or restore the call" >&2
    violations=1
    ;;
  esac
done <"$ALLOWLIST"

# Future-extractor tripwire: any body-extractor impl (FromRequest, not
# FromRequestParts) outside the known set must be explicitly reviewed.
if grep -rn "impl" "$WEB_API_SRC" --include='*.rs' | grep "FromRequest<" |
  grep -v "FromRequestParts<" |
  grep -vE "for (Validated|Unvalidated|UnvalidatedForm)<"; then
  echo "Unknown FromRequest (body extractor) impl above — review it and add it to this gate's known set deliberately" >&2
  violations=1
fi

if [ "$violations" -ne 0 ]; then
  exit 1
fi
echo "verify_no_raw_body_extractors: OK"
