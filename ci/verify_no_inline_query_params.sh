#!/usr/bin/env bash
# Verify that no route handler hand-maintains a query parameter as an inline
# `#[utoipa::path(params(("name" = Type, Query, ...)))]` tuple.
#
# Why: an inline Query tuple duplicates the handler's `Query<Struct>` extractor
# fields. The two can silently drift — and the `openapi_json_is_up_to_date`
# golden test CANNOT catch it (it compares committed-vs-regenerated, both wrong).
# This dropped the software-items name filter once. The fix is to derive the
# params from the struct: `params(SomeStruct)` where `SomeStruct: utoipa::IntoParams`.
# See docs/development/coding-standards.md ("OpenAPI parameter & schema authoring")
# and docs/adr/0025-drift-proof-openapi-params.md.
#
# Scope: crates/ui/web-api/src (*.rs).
# The pattern matches ONLY inline param tuples with the `Query` location keyword
# (a quoted name, then `=`, then a type, then `, Query`). It does NOT match
# `("id" = Uuid, Path, ...)` Path tuples, `Query<Struct>` extractors, or
# `use ...Query` imports (none have a quoted-string `=` before `, Query`).
#
# Limitation: matches line-by-line (rg, no --multiline), so a manually
# hand-wrapped tuple with `Query` on a separate line would be missed. In
# practice utoipa param tuples are single-line (rustfmt does not reformat
# attribute contents), so this has not occurred; the golden test + review
# remain the backstop for that edge.
#
# Allowlist a genuinely non-`Query<Struct>`-backed query param in
# ci/verify_no_inline_query_params_allowlist.txt (format: `path|text-regex`).
#
# Exit codes:
#   0 — no inline Query-param tuples (or all allowlisted)
#   1 — at least one inline Query-param tuple outside the allowlist
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SCAN_DIR="crates/ui/web-api/src"
ALLOWLIST_FILE="ci/verify_no_inline_query_params_allowlist.txt"
# Inline utoipa Query-param tuple: "<name>" = <type>, Query
PATTERN='"[^"]+"[[:space:]]*=[^,]+,[[:space:]]*Query'

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_no_inline_query_params: required tool 'rg' was not found"
  exit 1
fi

if [[ ! -f "$ALLOWLIST_FILE" ]]; then
  echo "verify_no_inline_query_params: allowlist file is missing: $ALLOWLIST_FILE"
  exit 1
fi

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

# Reject a malformed allowlist regex up front (matches the sibling verify_* idiom).
is_valid_ere() {
  local pattern="$1"
  printf '' | grep -E "$pattern" >/dev/null 2>&1 || [[ $? -eq 1 ]]
}

# Allowlist rows: `path|text-regex`. Blank lines and `#` comments ignored.
declare -a ALLOW_PATHS=()
declare -a ALLOW_TEXT_PATTERNS=()

load_allowlist() {
  local line line_no=0 raw path text_pattern
  while IFS= read -r line || [[ -n "$line" ]]; do
    line_no=$((line_no + 1))
    raw="${line%$'\r'}"
    [[ -z "$(trim "$raw")" ]] && continue
    [[ "$(trim "$raw")" == \#* ]] && continue

    if [[ "$raw" != *"|"* ]]; then
      echo "verify_no_inline_query_params: invalid allowlist row ${line_no}: expected 'path|text-regex'"
      exit 1
    fi
    path="$(trim "${raw%%|*}")"
    text_pattern="$(trim "${raw#*|}")"

    if [[ -z "$path" || -z "$text_pattern" ]]; then
      echo "verify_no_inline_query_params: invalid allowlist row ${line_no}: expected 'path|text-regex'"
      exit 1
    fi
    if [[ "$path" != crates/* ]]; then
      echo "verify_no_inline_query_params: allowlist path must start with 'crates/' at row ${line_no}"
      exit 1
    fi
    if ! is_valid_ere "$text_pattern"; then
      echo "verify_no_inline_query_params: invalid regex in allowlist row ${line_no}: ${text_pattern}"
      exit 1
    fi
    ALLOW_PATHS+=("$path")
    ALLOW_TEXT_PATTERNS+=("$text_pattern")
  done <"$ALLOWLIST_FILE"
}

is_allowlisted() {
  local path="$1" text="$2" idx
  for idx in "${!ALLOW_PATHS[@]}"; do
    [[ "${ALLOW_PATHS[$idx]}" == "$path" ]] || continue
    if printf '%s\n' "$text" | grep -Eq "${ALLOW_TEXT_PATTERNS[$idx]}"; then
      return 0
    fi
  done
  return 1
}

load_allowlist

declare -a VIOLATIONS=()

# rg exits 0 on matches, 1 on no matches (legal here), >=2 on error. Capture the
# status via a temp file: a `done < <(rg ...)` process substitution hides it, so
# a broken pattern or unreadable path would report an empty, green run.
RG_TMP="$(mktemp)"
RG_RC=0
rg -n --no-heading "$PATTERN" "$SCAN_DIR" --glob '**/*.rs' >"$RG_TMP" || RG_RC=$?
if (( RG_RC > 1 )); then
  echo "verify_no_inline_query_params: rg failed (rc=${RG_RC})" >&2
  rm -f "$RG_TMP"
  exit 1
fi

while IFS= read -r line; do
  path="${line%%:*}"
  rest="${line#*:}"
  line_no="${rest%%:*}"
  text="${rest#*:}"
  # Skip Rust comment lines (`//`, `/*`, `*`).
  case "$(trim "$text")" in
    '//'* | '/*'* | '*'*) continue ;;
  esac
  is_allowlisted "$path" "$text" && continue
  VIOLATIONS+=("${path}:${line_no}:${text}")
done <"$RG_TMP"
rm -f "$RG_TMP"

if (( ${#VIOLATIONS[@]} > 0 )); then
  echo "verify_no_inline_query_params: hand-maintained inline Query params found."
  echo "Derive them instead: params(SomeStruct) where SomeStruct: utoipa::IntoParams."
  echo "See docs/development/coding-standards.md + docs/adr/0025-drift-proof-openapi-params.md."
  echo ""
  printf '%s\n' "${VIOLATIONS[@]}"
  exit 1
fi

echo "verify_no_inline_query_params: OK"
